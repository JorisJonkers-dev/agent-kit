//! Thin entrypoint: the command logic lives in the library so the unit tests
//! can cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(pr_composer::run(std::env::args_os().skip(1)));
}
