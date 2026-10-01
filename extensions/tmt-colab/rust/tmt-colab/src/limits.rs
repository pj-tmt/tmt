//! L2 product bounds, distinct from the disposable #830 fixture budgets.
pub const OBJECT_BYTES: usize = 16 * 1024 * 1024 + 2 * 1024;
pub const PAGE_BYTES: usize = 64 * 1024 * 1024;
pub const PAGE_RECEIPTS: usize = 100_000;
