//! Online metadata sources beyond Crossref, arXiv and OpenAlex (projects.md
//! §4). Each module says at its top what leaves the machine; every fetch
//! checks `lookup::ensure_enabled` before it builds a request and sends
//! through the `HostLimiter`, so no service sees a burst. Parsers are pure
//! functions of the body text, tested on inline fixtures trimmed from each
//! API's documented response shape.

pub mod datacite;
pub mod dblp;
pub mod inspire;
pub mod openlibrary;
pub mod semantic_scholar;
