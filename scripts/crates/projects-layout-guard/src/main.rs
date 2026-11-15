//! Thin entrypoint: the hook logic lives in the library so the unit tests can
//! cover it; this stays the one executable line they cannot.

fn main() {
    std::process::exit(projects_layout_guard::run_main(
        std::io::stdin(),
        std::io::stdout(),
        |name| std::env::var(name).ok(),
    ));
}
