//! koyomi's engine-side model with no window in it: a job net's TOML and the
//! checks on it, the store of definitions a person has approved, the folder
//! scan that joins them into what the tree and `koyomi list` show, one run of
//! a net (its order and states), the day's history and `koyomi.toml`.

pub mod approved;
pub mod config;
pub mod duration;
pub mod history;
pub mod jobnet;
pub mod retain;
pub mod run;
pub mod scan;
pub mod when;
