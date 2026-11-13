//! The viewer of M0v in the browser (`docs/architecture.md`, 11.5), read-only: the same viewer as `apps/labcraft`, its
//! session of `engine` in a Web Worker, which a trap of WASM can stop without stopping the page.
//!
//! The library is what both ends need, and what is tested without a browser: what they say to each other
//! ([`protocol`]), what the worker does ([`host`]), what the page knows of its worker and does when it traps
//! ([`supervisor`]), the walk of a folder dropped on the page ([`folder`]), its address ([`link`]), the folders it
//! bundles. The page and the
//! worker themselves, in the browser only, carry bytes and draw.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::string_slice
)]
#![forbid(unsafe_code)]

pub mod folder;
pub mod host;
pub mod link;
#[cfg(target_arch = "wasm32")]
pub mod page;
pub mod protocol;
pub mod supervisor;
#[cfg(target_arch = "wasm32")]
pub mod worker;

use protocol::{File, Folder};

/// A folder that the page opens without a file of the user's: the examples, the programs of the RFCs.
#[derive(Debug)]
pub struct Bundled {
    pub name: &'static str,
    pub files: &'static [BundledFile],
}

#[derive(Debug)]
pub struct BundledFile {
    /// From the folder, with `/`.
    pub path: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

impl Bundled {
    /// As the page would read it.
    pub fn folder(&self) -> Folder {
        let files = self.files.iter().map(|file| File { path: file.path.to_owned(), bytes: file.bytes.to_vec() }).collect();
        Folder { name: self.name.to_owned(), files, folders: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use serde_json::json;

    use super::*;
    use crate::host::Host;
    use crate::protocol::{Answer, ToWorker, decode, encode};

    /// Every function of each bundled folder, opened and drawn by the worker, and its drawing read back by the page:
    /// natively, and in WASM under Node, with the stack of a worker (`cargo xtask wasm`).
    #[test]
    fn every_bundled_function_is_drawn_and_crosses_to_the_page() {
        assert_eq!(BUNDLED.iter().map(|bundled| bundled.name).collect::<Vec<_>>(), ["examples", "docs/rfc"]);
        let mut drawn = 0usize;
        for bundled in BUNDLED {
            let mut host = Host::new(None);
            let open = ToWorker::Open { folder: std::sync::Arc::new(bundled.folder()), standard_everywhere: false, skipped: Vec::new(), show: None };
            let answer: Answer = decode(&encode(&host.handle(open)).unwrap()).unwrap();
            assert!(answer.result.is_ok(), "{}: {:?}", bundled.name, answer.result);
            let listing = answer.listing.unwrap_or_else(|| panic!("{}: no listing", bundled.name));
            assert_eq!((listing.errors, listing.problems.as_str()), (0, ""), "{}", bundled.name);
            let functions: Vec<String> = listing.functions.iter().map(|listed| listed.function.clone()).collect();
            assert!(!functions.is_empty(), "{}", bundled.name);
            for function in functions {
                let message = ToWorker::Command { id: "view.show_function".to_owned(), params: json!({ "function": function }).to_string() };
                let answer: Answer = decode(&encode(&host.handle(message)).unwrap()).unwrap();
                assert!(answer.result.is_ok(), "{}: {function}: {:?}", bundled.name, answer.result);
                let drawing = answer.drawing.unwrap_or_else(|| panic!("{}: {function}: no drawing", bundled.name));
                assert!(!drawing.groups.is_empty(), "{}: {function}", bundled.name);
                drawn += 1;
            }
        }
        assert!(drawn > 15, "{drawn} functions");
    }

    /// The page opens without an address on a function that the bundled folders have.
    #[test]
    fn the_page_opens_on_a_bundled_function() {
        use crate::link::{DEFAULT_FOLDER, DEFAULT_FUNCTION};
        let bundled = BUNDLED.iter().find(|bundled| bundled.name == DEFAULT_FOLDER).unwrap();
        let open = ToWorker::Open {
            folder: std::sync::Arc::new(bundled.folder()),
            standard_everywhere: false,
            skipped: Vec::new(),
            show: Some(DEFAULT_FUNCTION.to_owned()),
        };
        let answer = Host::new(None).handle(open);
        assert_eq!(answer.state.function.as_deref(), Some(DEFAULT_FUNCTION));
        assert_eq!(answer.drawing.map(|drawing| drawing.function.clone()).as_deref(), Some(DEFAULT_FUNCTION));
    }
}
