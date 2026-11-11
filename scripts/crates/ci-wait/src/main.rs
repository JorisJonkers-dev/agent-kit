fn main() {
    std::process::exit(ci_wait::run(std::env::args_os().skip(1)));
}
