//! 구조 공유 persistent vector (XI.1.4).
//!
//! [`PersistentVec`]은 32갈래 trie(bit-partitioned vector trie)입니다. 원소는 크기 32 이하의
//! leaf chunk에, chunk는 크기 32 이하의 branch에 `Arc`로 담깁니다. [`PersistentVec::set`]은
//! root에서 해당 leaf까지의 경로(높이 h면 chunk h개, 각 32칸 이하)만 새로 할당하고 나머지
//! chunk는 이전 vector와 공유합니다(path copying). 이전 vector는 바뀌지 않습니다.
//!
//! scene node의 자식 목록과 snapshot의 최상위 node 목록이 이 타입을 써서, 자식이 수만 개인
//! frame에서도 자식 하나를 바꿀 때 O(log₃₂ n) chunk만 새로 만듭니다.
//!
//! chunk는 크기가 정해진 `Arc<Vec<_>>`로 담아 [`Arc::into_inner`]로 유일 소유 chunk를 넘겨받을
//! 수 있게 합니다. [`PersistentVec::release_unique`]는 이 성질로 경로 복사한 이전 version을
//! 해제할 때 그 version만 가진 chunk만 풀고 공유 chunk는 참조만 줄입니다.

use std::fmt;
use std::sync::Arc;

const BITS: u32 = 5;
const WIDTH: usize = 1 << BITS;
const MASK: usize = WIDTH - 1;

enum Chunk<T> {
    Leaf(Arc<Vec<T>>),
    Branch(Arc<Vec<Chunk<T>>>),
}

impl<T> Clone for Chunk<T> {
    fn clone(&self) -> Chunk<T> {
        match self {
            Chunk::Leaf(leaf) => Chunk::Leaf(Arc::clone(leaf)),
            Chunk::Branch(branch) => Chunk::Branch(Arc::clone(branch)),
        }
    }
}

/// 구조 공유 불변 vector.
pub struct PersistentVec<T> {
    len: usize,
    /// root에서 leaf까지 내려갈 때 첫 branch가 쓰는 bit shift(root가 leaf면 0).
    shift: u32,
    root: Option<Chunk<T>>,
}

impl<T> Clone for PersistentVec<T> {
    /// root 참조만 복제합니다(O(1)).
    fn clone(&self) -> PersistentVec<T> {
        PersistentVec {
            len: self.len,
            shift: self.shift,
            root: self.root.clone(),
        }
    }
}

impl<T> Default for PersistentVec<T> {
    fn default() -> PersistentVec<T> {
        PersistentVec::new()
    }
}

impl<T> PersistentVec<T> {
    /// 빈 vector.
    pub const fn new() -> PersistentVec<T> {
        PersistentVec {
            len: 0,
            shift: 0,
            root: None,
        }
    }

    /// 원소 수.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// 비어 있으면 참입니다.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `index`번째 원소.
    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        let mut chunk = self.root.as_ref()?;
        let mut shift = self.shift;
        loop {
            match chunk {
                Chunk::Branch(branch) => {
                    chunk = branch.get((index >> shift) & MASK)?;
                    shift = shift.checked_sub(BITS)?;
                }
                Chunk::Leaf(leaf) => return leaf.get(index & MASK),
            }
        }
    }

    /// 마지막 원소.
    pub fn last(&self) -> Option<&T> {
        self.len.checked_sub(1).and_then(|index| self.get(index))
    }

    /// 앞에서부터 순회합니다.
    pub fn iter(&self) -> Iter<'_, T> {
        let mut iter = Iter {
            stack: Vec::new(),
            leaf: &[],
            pos: 0,
        };
        match &self.root {
            None => {}
            Some(Chunk::Leaf(leaf)) => iter.leaf = leaf.as_slice(),
            Some(Chunk::Branch(branch)) => iter.stack.push((branch.as_slice(), 0)),
        }
        iter
    }

    /// 두 vector가 공유하는 leaf chunk 수(같은 위치의 같은 `Arc`). 구조 공유 진단용입니다.
    pub fn shared_leaf_chunks(&self, other: &PersistentVec<T>) -> usize {
        fn walk<T>(a: &Chunk<T>, b: &Chunk<T>) -> usize {
            match (a, b) {
                (Chunk::Leaf(x), Chunk::Leaf(y)) => usize::from(Arc::ptr_eq(x, y)),
                (Chunk::Branch(x), Chunk::Branch(y)) => {
                    if Arc::ptr_eq(x, y) {
                        x.iter().map(leaf_count).sum()
                    } else {
                        x.iter().zip(y.iter()).map(|(a, b)| walk(a, b)).sum()
                    }
                }
                _ => 0,
            }
        }
        fn leaf_count<T>(chunk: &Chunk<T>) -> usize {
            match chunk {
                Chunk::Leaf(_) => 1,
                Chunk::Branch(branch) => branch.iter().map(leaf_count).sum(),
            }
        }
        match (&self.root, &other.root) {
            (Some(a), Some(b)) if self.shift == other.shift => walk(a, b),
            _ => 0,
        }
    }

    /// vector를 해제하면서, 이 vector가 마지막 참조인 chunk의 원소만 `out`으로 옮깁니다.
    ///
    /// 다른 vector와 공유하는 chunk는 참조만 하나 줄이고 내려가지 않습니다. 그래서 경로 복사로
    /// 만든 이전 version을 해제하는 비용은 전체 길이가 아니라 그 version만 가진 chunk 수에
    /// 비례합니다(XI.1.4 비용 모델). chunk는 [`Arc::into_inner`]로 넘겨받으므로 같은 chunk를
    /// 여러 thread가 동시에 해제해도 정확히 하나만 원소를 받고, 원소는 복제하지 않고 옮깁니다.
    /// trie는 명시적 stack으로 순회합니다.
    ///
    /// 반환값은 참조를 놓은 chunk 수(내려간 chunk와 공유라서 멈춘 chunk 모두)입니다. 해제 비용
    /// 진단에 씁니다.
    pub fn release_unique(self, out: &mut Vec<T>) -> usize {
        let mut released = 0usize;
        let mut stack: Vec<Chunk<T>> = self.root.into_iter().collect();
        while let Some(chunk) = stack.pop() {
            released += 1;
            match chunk {
                Chunk::Leaf(leaf) => {
                    if let Some(items) = Arc::into_inner(leaf) {
                        out.extend(items);
                    }
                }
                Chunk::Branch(branch) => {
                    if let Some(chunks) = Arc::into_inner(branch) {
                        stack.extend(chunks);
                    }
                }
            }
        }
        released
    }
}

impl<T: Clone> PersistentVec<T> {
    /// `Vec`에서 만듭니다(chunk를 왼쪽부터 가득 채움).
    pub fn from_vec(items: Vec<T>) -> PersistentVec<T> {
        let len = items.len();
        if len == 0 {
            return PersistentVec::new();
        }
        let mut level: Vec<Chunk<T>> = items
            .chunks(WIDTH)
            .map(|chunk| Chunk::Leaf(Arc::new(chunk.to_vec())))
            .collect();
        let mut shift = 0;
        while level.len() > 1 {
            level = level
                .chunks(WIDTH)
                .map(|chunk| Chunk::Branch(Arc::new(chunk.to_vec())))
                .collect();
            shift += BITS;
        }
        PersistentVec {
            len,
            shift,
            root: level.pop(),
        }
    }

    /// `index`번째 원소를 바꾼 새 vector. 경로 밖의 chunk는 공유합니다.
    ///
    /// 범위 밖이면 `None`.
    pub fn set(&self, index: usize, value: T) -> Option<PersistentVec<T>> {
        if index >= self.len {
            return None;
        }
        fn replace<T: Clone>(
            chunk: &Chunk<T>,
            shift: u32,
            index: usize,
            value: T,
        ) -> Option<Chunk<T>> {
            match chunk {
                Chunk::Leaf(leaf) => {
                    let mut items: Vec<T> = leaf.to_vec();
                    *items.get_mut(index & MASK)? = value;
                    Some(Chunk::Leaf(Arc::new(items)))
                }
                Chunk::Branch(branch) => {
                    let slot = (index >> shift) & MASK;
                    let child = replace(branch.get(slot)?, shift.checked_sub(BITS)?, index, value)?;
                    let mut chunks: Vec<Chunk<T>> = branch.to_vec();
                    *chunks.get_mut(slot)? = child;
                    Some(Chunk::Branch(Arc::new(chunks)))
                }
            }
        }
        let root = replace(self.root.as_ref()?, self.shift, index, value)?;
        Some(PersistentVec {
            len: self.len,
            shift: self.shift,
            root: Some(root),
        })
    }

    /// 모든 원소를 `Vec`으로 복사합니다.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }
}

impl<T: Clone> FromIterator<T> for PersistentVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> PersistentVec<T> {
        PersistentVec::from_vec(iter.into_iter().collect())
    }
}

impl<T: PartialEq> PartialEq for PersistentVec<T> {
    /// 원소 값으로 비교합니다(공유 구조와 무관).
    fn eq(&self, other: &PersistentVec<T>) -> bool {
        self.len == other.len && self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl<T: Eq> Eq for PersistentVec<T> {}

impl<T: fmt::Debug> fmt::Debug for PersistentVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// [`PersistentVec::iter`] 반복자.
pub struct Iter<'a, T> {
    stack: Vec<(&'a [Chunk<T>], usize)>,
    leaf: &'a [T],
    pos: usize,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            if let Some(item) = self.leaf.get(self.pos) {
                self.pos += 1;
                return Some(item);
            }
            // 현재 leaf를 다 읽었으면 다음 leaf로 내려갑니다.
            let (branch, next) = self.stack.last_mut()?;
            match branch.get(*next) {
                None => {
                    self.stack.pop();
                }
                Some(chunk) => {
                    *next += 1;
                    match chunk {
                        Chunk::Leaf(leaf) => {
                            self.leaf = leaf.as_slice();
                            self.pos = 0;
                        }
                        Chunk::Branch(child) => self.stack.push((child.as_slice(), 0)),
                    }
                }
            }
        }
    }
}

impl<T> fmt::Debug for Iter<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Iter").field("pos", &self.pos).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_iter_and_len_agree_across_trie_heights() {
        // 높이 1(≤32), 2(≤1024), 3(≤32768), 4 경계를 모두 지납니다.
        for len in [
            0usize, 1, 31, 32, 33, 1023, 1024, 1025, 32_768, 32_769, 40_000,
        ] {
            let items: Vec<usize> = (0..len).collect();
            let vec = PersistentVec::from_vec(items.clone());
            assert_eq!(vec.len(), len);
            assert_eq!(vec.is_empty(), len == 0);
            assert_eq!(vec.to_vec(), items, "len {len}");
            for probe in [0, len / 2, len.saturating_sub(1)] {
                assert_eq!(vec.get(probe), items.get(probe), "len {len} probe {probe}");
            }
            assert_eq!(vec.get(len), None);
            assert_eq!(vec.last(), items.last());
        }
    }

    #[test]
    fn set_copies_only_the_path_and_keeps_the_old_vector() {
        let old: PersistentVec<u32> = (0..10_000u32).collect();
        let new = old.set(5_000, 99).expect("in range");
        assert_eq!(old.get(5_000), Some(&5_000));
        assert_eq!(new.get(5_000), Some(&99));
        assert_eq!(new.len(), old.len());
        for index in [0usize, 4_999, 5_001, 9_999] {
            assert_eq!(new.get(index), old.get(index));
        }
        // leaf chunk 313개 중 바뀐 1개만 새로 할당되고 나머지는 공유합니다.
        let leaves = 10_000usize.div_ceil(WIDTH);
        assert_eq!(new.shared_leaf_chunks(&old), leaves - 1);
        assert_eq!(old.shared_leaf_chunks(&old.clone()), leaves);
        assert!(old.set(10_000, 1).is_none());
        assert_ne!(old, new);
        assert_eq!(old, (0..10_000u32).collect::<PersistentVec<u32>>());
    }

    #[test]
    fn release_unique_stops_at_chunks_shared_with_another_version() {
        // 회귀(scene-node-drop-o-width-regression): 경로 복사한 이전 version 해제는 그 version만
        // 가진 경로 chunk만 풀고, 공유 chunk의 원소는 건드리지 않아야 합니다.
        const LEN: usize = 200_000;
        // 200,000개는 높이 4(leaf + branch 3단)이므로 경로의 유일 chunk는 root·branch 2개·leaf
        // 1개입니다. 놓는 chunk는 root와 유일 branch 3개의 자식(각 ≤32칸)을 넘지 않습니다.
        const PATH_BOUND: usize = 1 + 3 * WIDTH;
        let old: PersistentVec<usize> = (0..LEN).collect();
        let new = old.set(123_456, 7).expect("in range");
        let mut out = Vec::new();
        let released = old.release_unique(&mut out);
        assert!(released <= PATH_BOUND, "놓은 chunk {released}");
        // 바뀐 leaf 하나의 원소만 넘어옵니다.
        let leaf_start = 123_456 / WIDTH * WIDTH;
        let mut expected: Vec<usize> = (leaf_start..leaf_start + WIDTH).collect();
        out.sort_unstable();
        expected.sort_unstable();
        assert_eq!(out, expected);
        // 남은 version은 그대로입니다.
        assert_eq!(new.len(), LEN);
        assert_eq!(new.get(123_456), Some(&7));
        assert_eq!(new.get(LEN - 1), Some(&(LEN - 1)));

        // root를 공유하는 복제본은 root 참조 하나만 놓습니다.
        let mut out = Vec::new();
        assert_eq!(new.clone().release_unique(&mut out), 1);
        assert!(out.is_empty());

        // 마지막 version은 모든 원소를 복제 없이 넘깁니다.
        let mut out = Vec::new();
        let released = new.release_unique(&mut out);
        assert_eq!(out.len(), LEN);
        assert!(released > LEN / WIDTH);
        let mut expected: Vec<usize> = (0..LEN).collect();
        expected[123_456] = 7;
        out.sort_unstable();
        expected.sort_unstable();
        assert_eq!(out, expected);

        let mut out = Vec::new();
        assert_eq!(PersistentVec::<u8>::new().release_unique(&mut out), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn equality_and_debug_are_value_based() {
        let a: PersistentVec<i32> = vec![1, 2, 3].into_iter().collect();
        let b = PersistentVec::from_vec(vec![1, 2, 3]);
        assert_eq!(a, b);
        assert_eq!(format!("{a:?}"), "[1, 2, 3]");
        assert_ne!(a, PersistentVec::from_vec(vec![1, 2]));
        assert_eq!(PersistentVec::<i32>::default(), PersistentVec::new());
    }
}
