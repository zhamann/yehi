use clap::Parser;

#[derive(Parser)]
#[command(name = "yehi", version, about = "Fast GPU video rendering from a JSON spec.")]
struct Cli {}

fn main() {
    Cli::parse();
}
