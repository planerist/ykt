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
mod undo;
#[cfg(test)]
mod undo_tests;
mod map;
mod array;
#[cfg(test)]
mod map_array_tests;
#[cfg(test)]
mod update_tests;
mod xml_frag;
mod xml_elem;
mod xml_text;
mod xml;
mod output;
#[cfg(test)]
mod doc_tests;
#[cfg(test)]
mod error_tests;
#[cfg(test)]
mod attrs_tests;
#[cfg(test)]
mod nested_tests;

uniffi::setup_scaffolding!();
