//! Tests of the build tracker. The scenario replay is the parity proof: each
//! fixture's goldens come from the node runner (tests/regenerate-fixtures.mjs),
//! and the library must reproduce them byte for byte.

mod scenarios;
mod units;
