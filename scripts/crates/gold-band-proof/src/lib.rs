//! The headless proof harness for Gold Band workflows (issue #447, replacing
//! #373 and #348). See docs/gold-band-proof.md.

pub mod cli;
pub mod harness;
pub mod record;
pub mod refuse;
pub mod sha256;
pub mod stub;

#[cfg(test)]
mod tests;
