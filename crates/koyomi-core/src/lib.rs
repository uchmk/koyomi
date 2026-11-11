//! koyomi's engine-side model with no window in it: a job net's TOML and the
//! checks on it, the store of definitions a person has approved, the folder
//! scan that joins them into what the tree and `koyomi list` show, one run of
//! a net (its order and states), the day's history and `koyomi.toml`; and, for
//! the editor, the net written back as its file and laid out on the canvas.

pub mod approved;
pub mod backfill;
pub mod blackout;
pub mod bundle;
pub mod calendar;
pub mod chore;
pub mod clixml;
pub mod config;
pub mod duration;
pub mod group;
pub mod held;
pub mod history;
pub mod jobnet;
pub mod late;
pub mod layout;
pub mod lock;
pub mod msg;
pub mod next;
pub mod preflight;
pub mod quiet;
pub mod rename;
pub mod replace;
pub mod report;
pub mod retain;
pub mod role;
pub mod run;
pub mod scan;
pub mod standby;
pub mod task;
pub mod template;
pub mod users;
pub mod uses;
pub mod when;
pub mod write;
