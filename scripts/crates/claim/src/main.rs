//! Thin entrypoint: the answering lives in the library so the unit tests
//! can cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(claim::run(std::env::args().skip(1).collect()));
}
