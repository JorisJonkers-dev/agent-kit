//! Thin entrypoint: the helpers live in the library so the unit tests can
//! cover them; this stays the one executable line they cannot.

fn main() {
    std::process::exit(ci_helpers::run(std::env::args().skip(1).collect()));
}
