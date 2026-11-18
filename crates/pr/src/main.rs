//! `pr` binary entrypoint.

fn main() {
    std::process::exit(pr::run(std::env::args().skip(1)));
}
