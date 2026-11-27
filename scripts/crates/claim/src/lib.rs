//! claim: a ticket claim is the git ref `refs/claims/<key>` in a repository, taken with a
//! create-only push, so exactly one agent wins. The lease is recorded in the claim commit.

pub mod lease;
pub mod store;

#[cfg(test)]
mod tests;

/// Placeholder until a later piece implements the command line; it always
/// reports bad usage.
pub fn run(_args: Vec<String>) -> i32 {
    2
}
