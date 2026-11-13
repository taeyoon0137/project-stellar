//! generational GPU handle (XI.5.2).
//!
//! 공통 계층은 GPU resource를 [`GpuHandle`]로만 가리킵니다. handle은 `{ index, generation, epoch }`
//! 세 정수이며 native pointer·객체를 담지 않습니다(아키텍처 §10.1). native 객체는 backend가
//! [`HandleTable`]에 보관하고, handle은 그 table에서만 해석됩니다. scene은 GPU handle을 담지 않고
//! [`SceneResourceRef`](stellar_scene::SceneResourceRef) 값 식별자만 씁니다.
//!
//! - **generation**: slot을 비울 때마다 generation이 오릅니다. 파괴된 resource의 handle은 같은 slot이
//!   재사용돼도 generation이 달라 조회에 실패합니다(stale handle). generation이 넘칠 slot은 은퇴시켜
//!   다시 쓰지 않으므로 오래된 handle이 우연히 맞는 일이 없습니다.
//! - **epoch**: table(= device 인스턴스) 세대입니다. device lost 뒤
//!   [`HandleTable::invalidate_all`]은 epoch를 올려 이전 device의 handle을 모두 무효로 만듭니다.
//! - **kind**: [`BufferKind`]·[`TextureKind`] 표식으로 buffer handle과 texture handle을 타입 수준에서
//!   구분합니다.
//! - handle은 table만 발급합니다(공개 생성자 없음). 저장·FFI용 직렬화도 제공하지 않습니다(III.1.3
//!   `RuntimeEntityId`와 같은 원칙).
//!
//! 공개 생성자가 없어 table 밖에서 handle을 만들 수 없습니다.
//!
//! ```compile_fail
//! use stellar_renderer_core::gpu_handle::TextureHandle;
//! let forged = TextureHandle { index: 0, generation: std::num::NonZeroU32::MIN, epoch: 0 };
//! ```
//!
//! buffer handle을 texture handle 자리에 쓸 수 없습니다.
//!
//! ```compile_fail
//! use stellar_renderer_core::gpu_handle::{BufferHandle, BufferKind, HandleTable, TextureHandle};
//! let mut buffers: HandleTable<BufferKind, u32> = HandleTable::new(0);
//! let buffer: BufferHandle = buffers.insert(1).unwrap();
//! let _texture: TextureHandle = buffer;
//! ```

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;

mod sealed {
    pub trait Sealed {}
}

/// handle 종류 표식.
pub trait HandleKind: sealed::Sealed {
    /// 진단용 이름.
    const NAME: &'static str;
}

/// buffer handle 표식.
#[derive(Debug)]
pub enum BufferKind {}

/// texture handle 표식.
#[derive(Debug)]
pub enum TextureKind {}

impl sealed::Sealed for BufferKind {}
impl sealed::Sealed for TextureKind {}

impl HandleKind for BufferKind {
    const NAME: &'static str = "buffer";
}

impl HandleKind for TextureKind {
    const NAME: &'static str = "texture";
}

/// generational GPU handle.
pub struct GpuHandle<K> {
    index: u32,
    generation: NonZeroU32,
    epoch: u32,
    _kind: PhantomData<fn() -> K>,
}

/// GPU buffer handle.
pub type BufferHandle = GpuHandle<BufferKind>;

/// GPU texture handle.
pub type TextureHandle = GpuHandle<TextureKind>;

impl<K> GpuHandle<K> {
    /// slot index.
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// slot generation.
    pub const fn generation(&self) -> u32 {
        self.generation.get()
    }

    /// 발급한 table의 epoch.
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }
}

impl<K> Clone for GpuHandle<K> {
    fn clone(&self) -> GpuHandle<K> {
        *self
    }
}

impl<K> Copy for GpuHandle<K> {}

impl<K> PartialEq for GpuHandle<K> {
    fn eq(&self, other: &GpuHandle<K>) -> bool {
        self.index == other.index
            && self.generation == other.generation
            && self.epoch == other.epoch
    }
}

impl<K> Eq for GpuHandle<K> {}

impl<K> PartialOrd for GpuHandle<K> {
    fn partial_cmp(&self, other: &GpuHandle<K>) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K> Ord for GpuHandle<K> {
    fn cmp(&self, other: &GpuHandle<K>) -> std::cmp::Ordering {
        (self.epoch, self.index, self.generation).cmp(&(other.epoch, other.index, other.generation))
    }
}

impl<K> Hash for GpuHandle<K> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
        self.epoch.hash(state);
    }
}

impl<K: HandleKind> fmt::Debug for GpuHandle<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}#{}v{}e{}",
            K::NAME,
            self.index,
            self.generation,
            self.epoch
        )
    }
}

/// handle 조회 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleError {
    /// 다른 epoch(이전 device)가 발급한 handle입니다.
    WrongEpoch {
        /// handle epoch.
        handle: u32,
        /// table epoch.
        table: u32,
    },
    /// slot이 비었거나 재사용되어 generation이 다릅니다(파괴된 resource).
    Stale {
        /// handle 종류 이름.
        kind: &'static str,
        /// slot index.
        index: u32,
    },
    /// table에 없는 slot입니다.
    OutOfRange {
        /// slot index.
        index: u32,
    },
    /// slot index 공간(`u32`)을 다 썼습니다.
    Exhausted,
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandleError::WrongEpoch { handle, table } => {
                write!(f, "다른 device 세대의 handle입니다: {handle} != {table}")
            }
            HandleError::Stale { kind, index } => {
                write!(f, "파괴된 {kind} handle입니다(slot {index})")
            }
            HandleError::OutOfRange { index } => write!(f, "없는 handle slot {index}"),
            HandleError::Exhausted => f.write_str("handle slot 공간을 다 썼습니다"),
        }
    }
}

impl std::error::Error for HandleError {}

#[derive(Debug)]
struct Slot<T> {
    generation: NonZeroU32,
    value: Option<T>,
}

/// generation 기반 handle table. backend가 native 객체를 보관합니다.
#[derive(Debug)]
pub struct HandleTable<K, T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    epoch: u32,
    live: usize,
    _kind: PhantomData<fn() -> K>,
}

impl<K: HandleKind, T> HandleTable<K, T> {
    /// 빈 table. `epoch`는 device 인스턴스 세대입니다.
    pub fn new(epoch: u32) -> HandleTable<K, T> {
        HandleTable {
            slots: Vec::new(),
            free: Vec::new(),
            epoch,
            live: 0,
            _kind: PhantomData,
        }
    }

    /// table epoch.
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// 살아 있는 entry 수.
    pub const fn len(&self) -> usize {
        self.live
    }

    /// entry가 없으면 참입니다.
    pub const fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// 값을 보관하고 handle을 발급합니다. 빈 slot을 먼저 재사용합니다(작은 index 우선은 보장하지
    /// 않음).
    pub fn insert(&mut self, value: T) -> Result<GpuHandle<K>, HandleError> {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            let generation = slot.generation;
            self.live += 1;
            return Ok(self.handle(index, generation));
        }
        let index = u32::try_from(self.slots.len()).map_err(|_| HandleError::Exhausted)?;
        self.slots.push(Slot {
            generation: NonZeroU32::MIN,
            value: Some(value),
        });
        self.live += 1;
        Ok(self.handle(index, NonZeroU32::MIN))
    }

    const fn handle(&self, index: u32, generation: NonZeroU32) -> GpuHandle<K> {
        GpuHandle {
            index,
            generation,
            epoch: self.epoch,
            _kind: PhantomData,
        }
    }

    fn slot_index(&self, handle: GpuHandle<K>) -> Result<usize, HandleError> {
        if handle.epoch != self.epoch {
            return Err(HandleError::WrongEpoch {
                handle: handle.epoch,
                table: self.epoch,
            });
        }
        let index = handle.index as usize;
        let slot = self.slots.get(index).ok_or(HandleError::OutOfRange {
            index: handle.index,
        })?;
        if slot.generation != handle.generation || slot.value.is_none() {
            return Err(HandleError::Stale {
                kind: K::NAME,
                index: handle.index,
            });
        }
        Ok(index)
    }

    /// handle이 가리키는 값.
    pub fn get(&self, handle: GpuHandle<K>) -> Result<&T, HandleError> {
        let index = self.slot_index(handle)?;
        self.slots[index].value.as_ref().ok_or(HandleError::Stale {
            kind: K::NAME,
            index: handle.index,
        })
    }

    /// handle이 가리키는 값(가변).
    pub fn get_mut(&mut self, handle: GpuHandle<K>) -> Result<&mut T, HandleError> {
        let index = self.slot_index(handle)?;
        self.slots[index].value.as_mut().ok_or(HandleError::Stale {
            kind: K::NAME,
            index: handle.index,
        })
    }

    /// handle이 유효하면 참입니다.
    pub fn contains(&self, handle: GpuHandle<K>) -> bool {
        self.slot_index(handle).is_ok()
    }

    /// 값을 꺼내고 slot generation을 올립니다. 이 handle과 복사본은 이후 모두 stale입니다.
    pub fn remove(&mut self, handle: GpuHandle<K>) -> Result<T, HandleError> {
        let index = self.slot_index(handle)?;
        let slot = &mut self.slots[index];
        let value = slot.value.take().ok_or(HandleError::Stale {
            kind: K::NAME,
            index: handle.index,
        })?;
        self.live -= 1;
        // generation이 넘치면 slot을 은퇴시켜 재사용하지 않습니다.
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(handle.index);
        }
        Ok(value)
    }

    /// 모든 값을 꺼내고 epoch를 올립니다(device lost). 이전 handle은 모두 `WrongEpoch`가 됩니다.
    ///
    /// epoch가 넘치면 `Exhausted`를 돌려주고 table을 바꾸지 않습니다.
    pub fn invalidate_all(&mut self) -> Result<Vec<T>, HandleError> {
        let epoch = self.epoch.checked_add(1).ok_or(HandleError::Exhausted)?;
        let values = self.slots.drain(..).filter_map(|slot| slot.value).collect();
        self.free.clear();
        self.live = 0;
        self.epoch = epoch;
        Ok(values)
    }

    /// 살아 있는 entry(index 순).
    pub fn iter(&self) -> impl Iterator<Item = (GpuHandle<K>, &T)> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.value
                .as_ref()
                .map(|value| (self.handle(index as u32, slot.generation), value))
        })
    }

    /// 테스트용: slot generation을 강제로 설정합니다.
    #[cfg(test)]
    fn force_generation(&mut self, handle: GpuHandle<K>, generation: NonZeroU32) -> GpuHandle<K> {
        let slot = &mut self.slots[handle.index as usize];
        slot.generation = generation;
        self.handle(handle.index, generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// backend가 보관하는 native 객체 모사(pointer 크기 값).
    #[derive(Debug, PartialEq, Eq)]
    struct NativeTexture {
        pointer: usize,
    }

    #[test]
    fn stale_handle_fixture_fails() {
        // 검증 기준(XI.5.2): stale handle fixture 실패.
        let mut table: HandleTable<TextureKind, NativeTexture> = HandleTable::new(0);
        let first = table.insert(NativeTexture { pointer: 0x1000 }).unwrap();
        assert_eq!(table.get(first).unwrap().pointer, 0x1000);
        let copy = first;
        assert_eq!(table.remove(first).unwrap().pointer, 0x1000);

        // 파괴 직후 원본과 복사본 모두 stale.
        let stale = HandleError::Stale {
            kind: "texture",
            index: first.index(),
        };
        assert_eq!(table.get(first), Err(stale));
        assert_eq!(table.get(copy), Err(stale));
        assert_eq!(table.remove(copy), Err(stale));
        assert!(!table.contains(first));

        // 같은 slot이 재사용돼도 generation이 달라 이전 handle은 새 값을 가리키지 않습니다.
        let second = table.insert(NativeTexture { pointer: 0x2000 }).unwrap();
        assert_eq!(second.index(), first.index());
        assert_ne!(second.generation(), first.generation());
        assert_ne!(second, first);
        assert_eq!(table.get(first), Err(stale));
        assert_eq!(table.get(second).unwrap().pointer, 0x2000);
        table.get_mut(second).unwrap().pointer = 0x2100;
        assert_eq!(table.get(second).unwrap().pointer, 0x2100);
        assert_eq!(table.len(), 1);

        // 다른 table에서 온 범위 밖 handle.
        let mut other: HandleTable<TextureKind, NativeTexture> = HandleTable::new(0);
        for pointer in 0..4 {
            other.insert(NativeTexture { pointer }).unwrap();
        }
        let foreign = other.insert(NativeTexture { pointer: 9 }).unwrap();
        assert_eq!(
            table.get(foreign),
            Err(HandleError::OutOfRange { index: 4 })
        );
    }

    #[test]
    fn device_reset_invalidates_every_handle() {
        let mut table: HandleTable<BufferKind, u32> = HandleTable::new(3);
        let a = table.insert(10).unwrap();
        let b = table.insert(20).unwrap();
        assert_eq!(a.epoch(), 3);
        let mut released = table.invalidate_all().unwrap();
        released.sort_unstable();
        assert_eq!(released, vec![10, 20]);
        assert_eq!(table.epoch(), 4);
        assert!(table.is_empty());
        for old in [a, b] {
            assert_eq!(
                table.get(old),
                Err(HandleError::WrongEpoch {
                    handle: 3,
                    table: 4
                })
            );
        }
        // 새 epoch의 첫 handle은 이전 handle과 index·generation이 같아도 다릅니다.
        let fresh = table.insert(30).unwrap();
        assert_eq!(
            (fresh.index(), fresh.generation()),
            (a.index(), a.generation())
        );
        assert_ne!(fresh, a);
        assert_eq!(
            table.get(a).unwrap_err(),
            HandleError::WrongEpoch {
                handle: 3,
                table: 4
            }
        );

        let mut last: HandleTable<BufferKind, u32> = HandleTable::new(u32::MAX);
        last.insert(1).unwrap();
        assert_eq!(last.invalidate_all(), Err(HandleError::Exhausted));
        assert_eq!(last.len(), 1);
    }

    #[test]
    fn exhausted_generation_retires_the_slot() {
        let mut table: HandleTable<TextureKind, u8> = HandleTable::new(0);
        let handle = table.insert(1).unwrap();
        let worn = table.force_generation(handle, NonZeroU32::MAX);
        assert_eq!(table.remove(worn), Ok(1));
        // 은퇴한 slot은 재사용하지 않습니다.
        let next = table.insert(2).unwrap();
        assert_ne!(next.index(), worn.index());
        assert_eq!(
            table.get(worn),
            Err(HandleError::Stale {
                kind: "texture",
                index: worn.index()
            })
        );
    }

    #[test]
    fn handles_carry_no_native_pointer() {
        // handle은 정수 세 개뿐입니다(포인터 크기와 무관).
        assert_eq!(std::mem::size_of::<TextureHandle>(), 12);
        // generation이 0이 아니므로 Option<handle>도 같은 크기입니다.
        assert_eq!(std::mem::size_of::<Option<TextureHandle>>(), 12);
        let mut table: HandleTable<TextureKind, NativeTexture> = HandleTable::new(0);
        let a = table
            .insert(NativeTexture {
                pointer: 0xdead_beef,
            })
            .unwrap();
        let b = table.insert(NativeTexture { pointer: 0xfeed }).unwrap();
        assert_eq!(format!("{a:?}"), "texture#0v1e0");
        let listed: Vec<(TextureHandle, usize)> = table
            .iter()
            .map(|(handle, native)| (handle, native.pointer))
            .collect();
        assert_eq!(listed, vec![(a, 0xdead_beef), (b, 0xfeed)]);
        assert!(a < b);
        let errors = [
            HandleError::WrongEpoch {
                handle: 1,
                table: 2,
            },
            HandleError::Stale {
                kind: "buffer",
                index: 0,
            },
            HandleError::OutOfRange { index: 3 },
            HandleError::Exhausted,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
