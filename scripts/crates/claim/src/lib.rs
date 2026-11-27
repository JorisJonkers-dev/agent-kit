//! claim: claim a device for one run by storing a lease in a git commit
//! message. This member is deliberately trivial and std-only, with no
//! dependency of any kind. New Rust joins this workspace as a new member,
//! never as its own Cargo project.

pub mod lease;

#[cfg(test)]
mod tests;

/// Placeholder until a later piece implements the command line; it always
/// reports bad usage.
pub fn run(_args: Vec<String>) -> i32 {
    2
}
