//! Bounded JSON arrays stop before growing beyond their contract cap.
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct List<T, const N: usize>(Vec<T>);
impl<T, const N: usize> Default for List<T, N> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<T, const N: usize> List<T, N> {
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for List<T, N> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Array<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Array<T, N> {
            type Value = List<T, N>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "at most {N} entries")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if seq.size_hint().is_some_and(|n| n > N) {
                    return Err(de::Error::custom("array exceeds cap"));
                }
                let mut out = Vec::new();
                while out.len() < N {
                    match seq.next_element()? {
                        Some(v) => out.push(v),
                        None => return Ok(List(out)),
                    }
                }
                if seq.next_element::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("array exceeds cap"));
                }
                Ok(List(out))
            }
        }
        d.deserialize_seq(Array::<T, N>(PhantomData))
    }
}
