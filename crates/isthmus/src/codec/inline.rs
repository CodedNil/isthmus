//! A fixed-capacity vector that crosses the host/shader boundary by value.
use super::ShaderData;
use core::ops::Index;

/// A fixed-capacity vector that can be shared with shaders.
#[derive(Clone, Copy, crate::ShaderData)]
pub struct InlineVec<T, const N: usize> {
    len: u32,
    items: [T; N],
}

impl<T: ShaderData, const N: usize> InlineVec<T, N> {
    pub const fn new() -> Self {
        Self { len: 0, items: [T::ZERO; N] }
    }

    pub const fn len(&self) -> usize {
        let len = self.len as usize;
        if len < N { len } else { N }
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn push(&mut self, item: T) {
        assert!((self.len as usize) < N, "inline vector is full");
        self.items[self.len as usize] = item;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len()]
    }
}

impl<T: ShaderData, const N: usize> Index<usize> for InlineVec<T, N> {
    type Output = T;

    fn index(&self, index: usize) -> &T {
        assert!(index < self.len(), "inline vector index is out of bounds");
        &self.items[index]
    }
}

impl<T: ShaderData, const N: usize> Default for InlineVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_vectors_bound_decoded_lengths_and_round_trip_active_items() {
        let mut value = InlineVec::<u32, 2>::new();
        value.push(7);
        let mut words = Vec::new();
        value.append(&mut words);
        assert_eq!(InlineVec::<u32, 2>::read(&words, 0).as_slice(), &[7]);
        words[0] = u32::MAX;
        assert_eq!(InlineVec::<u32, 2>::read(&words, 0).as_slice(), &[7, 0]);
        assert!(InlineVec::<u32, 0>::read(&[u32::MAX], 0).is_empty());
    }
}
