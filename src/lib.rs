#![forbid(unsafe_code)]

pub mod ai;
pub mod approval;
pub mod bls;
pub mod catalog;
pub mod dry_run;
pub mod error;
pub mod feedback;
pub mod leftovers;
pub mod mealie;
pub mod models;
pub mod nutrition;
pub mod planning_cycle;
pub mod profile;
pub mod publish;
pub mod review;
pub mod revision;
pub mod shopping;
pub mod storage;
pub mod units;
pub mod validation;

pub use error::{Error, Result};
