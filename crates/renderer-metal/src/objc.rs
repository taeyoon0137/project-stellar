//! 최소 Objective-C runtime FFI.
//!
//! 새 crate 의존 없이 `libobjc`의 C 함수(`objc_msgSend`, `sel_registerName`, `objc_getClass`,
//! retain·release, autorelease pool)만 선언해 Metal·QuartzCore 객체를 다룹니다(`project` crate의
//! CoreText·FSEvents adapter와 같은 방식).
//!
//! - **메시지 전송**: [`send`]는 `objc_msgSend`를 호출 지점의 정확한 C 시그니처로 바꿔 부릅니다.
//!   Apple Silicon(arm64, ADR-0006)만 대상이므로 struct 반환용 `objc_msgSend_stret`이 필요 없습니다.
//!   인자·반환 타입이 selector의 실제 시그니처와 다르면 정의되지 않은 동작이므로, 모든 호출은 이
//!   crate 안의 typed wrapper에서만 합니다.
//! - **소유권**: [`Id`]는 retain한 참조 하나를 소유하고 drop 시 release합니다. `new`·`alloc`·`copy`
//!   계열 반환값(+1)은 [`Id::from_owned`], 그 밖의 반환값(+0)은 [`Id::from_borrowed`]로 감쌉니다.
//! - **autorelease**: backend 메서드마다 [`autoreleasepool`]로 감싸 +0 임시 객체가 render thread에
//!   쌓이지 않게 합니다.
//! - **selector·class**: [`sel!`]·[`class!`]는 이름 조회 결과를 정적 cache에 둡니다(hot path 조회 비용
//!   제거).

#[cfg(test)]
use std::ffi::CStr;
use std::ffi::{c_char, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

/// retain 여부를 따지지 않은 원시 객체 포인터.
pub(crate) type RawId = *mut c_void;

/// `NSUInteger`.
pub(crate) type NSUInteger = usize;

/// Objective-C selector.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sel(*const c_void);

/// Objective-C class 객체.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Class(*const c_void);

impl Class {
    /// class 객체를 receiver 포인터로 봅니다(class method 호출용).
    pub(crate) fn as_raw(self) -> RawId {
        self.0.cast_mut()
    }
}

#[link(name = "objc", kind = "dylib")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> *const c_void;
    fn sel_registerName(name: *const c_char) -> *const c_void;
    fn objc_msgSend();
    fn objc_retain(obj: RawId) -> RawId;
    fn objc_release(obj: RawId);
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

// Foundation(NSString 등) class를 runtime에 올립니다.
#[link(name = "Foundation", kind = "framework")]
extern "C" {}

/// 이름 조회 결과를 보관하는 selector cache.
pub(crate) struct SelCache {
    name: &'static str,
    ptr: AtomicPtr<c_void>,
}

impl SelCache {
    /// NUL로 끝나는 selector 이름으로 만듭니다.
    pub(crate) const fn new(name: &'static str) -> SelCache {
        SelCache {
            name,
            ptr: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// selector. 처음 한 번만 runtime에 등록·조회합니다.
    pub(crate) fn get(&self) -> Sel {
        let cached = self.ptr.load(Ordering::Acquire);
        if !cached.is_null() {
            return Sel(cached);
        }
        debug_assert!(self.name.ends_with('\0'));
        // SAFETY: name은 `sel!`이 만든 NUL 종료 정적 문자열입니다.
        let registered = unsafe { sel_registerName(self.name.as_ptr().cast()) };
        self.ptr.store(registered.cast_mut(), Ordering::Release);
        Sel(registered)
    }
}

/// 이름 조회 결과를 보관하는 class cache.
pub(crate) struct ClassCache {
    name: &'static str,
    ptr: AtomicPtr<c_void>,
}

impl ClassCache {
    /// NUL로 끝나는 class 이름으로 만듭니다.
    pub(crate) const fn new(name: &'static str) -> ClassCache {
        ClassCache {
            name,
            ptr: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// class. framework에 없으면 `None`.
    pub(crate) fn get(&self) -> Option<Class> {
        let cached = self.ptr.load(Ordering::Acquire);
        if !cached.is_null() {
            return Some(Class(cached));
        }
        debug_assert!(self.name.ends_with('\0'));
        // SAFETY: name은 `class!`가 만든 NUL 종료 정적 문자열입니다.
        let found = unsafe { objc_getClass(self.name.as_ptr().cast()) };
        if found.is_null() {
            return None;
        }
        self.ptr.store(found.cast_mut(), Ordering::Release);
        Some(Class(found))
    }
}

/// 정적 cache된 selector.
macro_rules! sel {
    ($name:literal) => {{
        static CACHE: $crate::objc::SelCache = $crate::objc::SelCache::new(concat!($name, "\0"));
        CACHE.get()
    }};
}

/// 정적 cache된 class(`Option<Class>`).
macro_rules! class {
    ($name:literal) => {{
        static CACHE: $crate::objc::ClassCache =
            $crate::objc::ClassCache::new(concat!($name, "\0"));
        CACHE.get()
    }};
}

pub(crate) use {class, sel};

/// 메시지 인자 묶음(tuple).
pub(crate) trait MsgArgs {
    /// `objc_msgSend`를 이 인자 타입과 반환 타입 `R`의 C 시그니처로 부릅니다.
    ///
    /// # Safety
    ///
    /// `obj`는 유효한 객체(또는 class)이고, selector의 실제 시그니처가 `(self, _cmd, 인자...) -> R`과
    /// 정확히 같아야 합니다.
    unsafe fn invoke<R>(self, obj: RawId, sel: Sel) -> R;
}

macro_rules! impl_msg_args {
    ($($name:ident),*) => {
        impl<$($name: Copy),*> MsgArgs for ($($name,)*) {
            #[allow(non_snake_case)]
            unsafe fn invoke<R>(self, obj: RawId, sel: Sel) -> R {
                let ($($name,)*) = self;
                // SAFETY: objc_msgSend는 receiver·selector 뒤 인자를 그대로 대상 method에 넘기는
                // trampoline입니다. 호출자가 실제 시그니처 일치를 보장합니다(trait 문서).
                let function: unsafe extern "C" fn(RawId, Sel $(, $name)*) -> R =
                    unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
                // SAFETY: 위와 같습니다.
                unsafe { function(obj, sel $(, $name)*) }
            }
        }
    };
}

impl_msg_args!();
impl_msg_args!(A);
impl_msg_args!(A, B);
impl_msg_args!(A, B, C);
impl_msg_args!(A, B, C, D);
impl_msg_args!(A, B, C, D, E);
impl_msg_args!(A, B, C, D, E, F);
impl_msg_args!(A, B, C, D, E, F, G);
impl_msg_args!(A, B, C, D, E, F, G, H);
impl_msg_args!(A, B, C, D, E, F, G, H, I);

/// 메시지를 보냅니다.
///
/// # Safety
///
/// [`MsgArgs::invoke`]와 같습니다.
pub(crate) unsafe fn send<A: MsgArgs, R>(obj: RawId, sel: Sel, args: A) -> R {
    // SAFETY: 호출자 보장(함수 문서).
    unsafe { args.invoke(obj, sel) }
}

/// retain한 Objective-C 객체 참조. drop 시 release합니다.
#[derive(Debug)]
pub(crate) struct Id(NonNull<c_void>);

impl Id {
    /// +1 반환값(`new*`·`alloc`/`init`·`copy`·Create 규칙)의 소유권을 넘겨받습니다. null이면 `None`.
    ///
    /// # Safety
    ///
    /// `raw`는 null이거나, 호출자가 release 책임을 가진 유효한 객체여야 합니다.
    pub(crate) unsafe fn from_owned(raw: RawId) -> Option<Id> {
        NonNull::new(raw).map(Id)
    }

    /// +0 반환값을 retain해 소유합니다. null이면 `None`.
    ///
    /// # Safety
    ///
    /// `raw`는 null이거나 지금 살아 있는 유효한 객체여야 합니다.
    pub(crate) unsafe fn from_borrowed(raw: RawId) -> Option<Id> {
        let raw = NonNull::new(raw)?;
        // SAFETY: raw는 살아 있는 객체입니다(호출자 보장). retain은 같은 포인터를 돌려줍니다.
        unsafe { objc_retain(raw.as_ptr()) };
        Some(Id(raw))
    }

    /// 원시 포인터(소유권 이동 없음).
    pub(crate) fn as_raw(&self) -> RawId {
        self.0.as_ptr()
    }

    /// 이 객체에 메시지를 보냅니다.
    ///
    /// # Safety
    ///
    /// [`MsgArgs::invoke`]와 같습니다(receiver는 이 객체).
    pub(crate) unsafe fn send<A: MsgArgs, R>(&self, sel: Sel, args: A) -> R {
        // SAFETY: self는 살아 있는 객체이고 시그니처 일치는 호출자 보장입니다.
        unsafe { send(self.as_raw(), sel, args) }
    }
}

impl Clone for Id {
    fn clone(&self) -> Id {
        // SAFETY: self는 살아 있는 객체이므로 retain해 참조를 하나 더 만듭니다.
        unsafe { objc_retain(self.as_raw()) };
        Id(self.0)
    }
}

impl Drop for Id {
    fn drop(&mut self) {
        // SAFETY: 이 값이 소유한 참조 하나를 정확히 한 번 release합니다.
        unsafe { objc_release(self.as_raw()) };
    }
}

/// `f`를 autorelease pool 안에서 실행합니다. panic으로 빠져나와도 pool을 닫습니다.
pub(crate) fn autoreleasepool<T>(f: impl FnOnce() -> T) -> T {
    struct Pool(*mut c_void);
    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: push가 돌려준 pool token을 같은 thread에서 정확히 한 번 pop합니다.
            unsafe { objc_autoreleasePoolPop(self.0) };
        }
    }
    // SAFETY: 인자 없는 runtime 호출입니다.
    let _pool = Pool(unsafe { objc_autoreleasePoolPush() });
    f()
}

/// `NSUTF8StringEncoding`.
const NS_UTF8_STRING_ENCODING: NSUInteger = 4;

/// UTF-8 문자열로 `NSString`을 만듭니다(+1). 실패하면 `None`.
pub(crate) fn ns_string(text: &str) -> Option<Id> {
    let class = class!("NSString")?;
    // SAFETY: `+[NSString alloc]`은 (Class) -> id, `-initWithBytes:length:encoding:`은
    // (const void*, NSUInteger, NSUInteger) -> id(+1, 실패 시 nil)입니다. bytes는 len만큼 읽기 가능합니다.
    unsafe {
        let allocated: RawId = send(class.as_raw(), sel!("alloc"), ());
        let initialized: RawId = send(
            allocated,
            sel!("initWithBytes:length:encoding:"),
            (
                text.as_ptr().cast::<c_void>(),
                text.len() as NSUInteger,
                NS_UTF8_STRING_ENCODING,
            ),
        );
        Id::from_owned(initialized)
    }
}

/// `NSString`을 최대 `max_bytes` byte의 Rust 문자열로 옮깁니다(UTF-8 경계에서 자름).
///
/// `string`은 null이거나 유효한 `NSString`이어야 합니다(호출자가 selector 반환값을 그대로 넘김).
#[cfg(test)]
pub(crate) fn rust_string(string: RawId, max_bytes: usize) -> Option<String> {
    if string.is_null() {
        return None;
    }
    // SAFETY: string은 유효한 NSString이고 `-UTF8String`은 () -> const char*(내부 버퍼, pool 동안
    // 유효, 실패 시 NULL)입니다.
    let raw: *const c_char = unsafe { send(string, sel!("UTF8String"), ()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: UTF8String은 NUL 종료 문자열을 돌려줍니다.
    let text = unsafe { CStr::from_ptr(raw) }.to_string_lossy();
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(text[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ns_string_round_trips_and_truncates_on_char_boundary() {
        autoreleasepool(|| {
            let string = ns_string("셰이더 log").expect("NSString");
            assert_eq!(
                rust_string(string.as_raw(), 1024).as_deref(),
                Some("셰이더 log")
            );
            // "셰"(3 byte) 중간에서 자르지 않습니다.
            assert_eq!(rust_string(string.as_raw(), 4).as_deref(), Some("셰"));
            assert_eq!(rust_string(std::ptr::null_mut(), 8), None);
        });
    }

    #[test]
    fn missing_class_is_none_and_clone_balances_retain() {
        assert!(class!("StellarNoSuchClass").is_none());
        autoreleasepool(|| {
            // tagged pointer가 되지 않도록 비 ASCII 긴 문자열을 씁니다.
            let string = ns_string("retain 균형 확인용 문자열").expect("NSString");
            // SAFETY: `-retainCount`는 () -> NSUInteger입니다(진단용).
            let before: NSUInteger = unsafe { string.send(sel!("retainCount"), ()) };
            let copy = string.clone();
            // SAFETY: 위와 같습니다.
            let during: NSUInteger = unsafe { string.send(sel!("retainCount"), ()) };
            drop(copy);
            // SAFETY: 위와 같습니다.
            let after: NSUInteger = unsafe { string.send(sel!("retainCount"), ()) };
            assert_eq!(during, before + 1);
            assert_eq!(after, before);
        });
    }
}
