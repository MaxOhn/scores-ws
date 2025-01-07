use std::sync::atomic::{AtomicPtr, Ordering};

use eyre::{Context, ContextCompat, Result};

pub struct Authorization {
    // We use an atomic pointer to allow mutation through immutable reference.
    // Since atomic pointers only support thin pointers, we need to keep the
    // str boxed which means double indirection but that's fine.
    ptr: AtomicPtr<Box<str>>,
}

impl Authorization {
    pub fn as_str(&self) -> &str {
        let ptr = self.ptr.load(Ordering::Acquire);

        unsafe { (*ptr).as_ref() }
    }

    pub fn parse(&self, bytes: &[u8]) -> Result<()> {
        const KEY: &[u8] = br#""access_token":"#;

        let idx = memchr::memmem::find(bytes, KEY).context("missing `\"access_token\"`")?;
        let bytes = &bytes[idx + KEY.len()..];
        let mut iter = memchr::memchr_iter(b'"', bytes);
        let (start, end) = iter.next().zip(iter.next()).context("missing quotes")?;

        let token = std::str::from_utf8(&bytes[start + 1..end])
            .context("access token is not valid utf-8")?;

        let authorization = format!("Bearer {token}");
        let ptr = Box::into_raw(Box::new(authorization.into_boxed_str()));

        let old = self.ptr.swap(ptr, Ordering::Release);
        unsafe { old.drop_in_place() };

        Ok(())
    }
}

impl Default for Authorization {
    fn default() -> Self {
        Self {
            ptr: AtomicPtr::new(Box::into_raw(Box::default())),
        }
    }
}

impl Drop for Authorization {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::Acquire);
        unsafe { ptr.drop_in_place() };
    }
}
