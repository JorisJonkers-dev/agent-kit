//! Thin entrypoint: the answering lives in the library so the unit tests
//! can cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(agent_kit_setup::run(std::env::args_os().skip(1)));
}
