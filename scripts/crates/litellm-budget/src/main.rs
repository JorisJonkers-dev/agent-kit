//! Thin entrypoint: the work lives in the library so the unit tests cover it.

fn main() {
    std::process::exit(litellm_budget::run(std::env::args_os().skip(1)));
}
