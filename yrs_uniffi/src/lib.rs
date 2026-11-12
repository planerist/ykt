mod attrs;
mod collection;
mod doc;
mod snapshots;
mod text;
mod tools;
mod transaction;
mod delta;
mod observers;
#[cfg(test)]
mod observer_tests;
mod updates;
mod sticky;
#[cfg(test)]
mod sticky_tests;
#[cfg(test)]
mod update_tests;
mod xml_frag;
mod xml_elem;
mod xml_text;
mod xml;

uniffi::setup_scaffolding!();
