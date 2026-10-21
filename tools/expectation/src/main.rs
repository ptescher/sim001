//! `cargo run -p expectation -- check expectations`: the expectation command, built here so the
//! workspace's tasks can run it.

fn main() {
    expectation_cli::main();
}
