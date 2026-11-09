//! Thin entrypoint: the checking lives in the library so the unit tests can
//! cover it; this stays the one executable line they cannot. mount is the
//! one command with an outside effect, so the entrypoint hands `run` the
//! real (untested, few-line) Outside impl.

fn main() {
    std::process::exit(research_layout::run(
        std::env::args_os().skip(1),
        &mut research_layout::RealOutside,
    ));
}
