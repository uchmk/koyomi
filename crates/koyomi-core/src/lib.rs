//! koyomi's engine-side model with no window in it: a job net's TOML and the
//! checks on it, the store of definitions a person has approved, and the
//! folder scan that joins them into what the tree and `koyomi list` show.

pub mod approved;
pub mod jobnet;
pub mod scan;
