use std::ops::Deref;

mod attrs;
mod collection;
mod doc;
mod js;
mod snapshots;
mod text;
mod tools;
mod transaction;

uniffi::setup_scaffolding!();
