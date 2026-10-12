use clap::Parser;

/// Dijkstral turns a repository into a queryable knowledge graph for AI coding agents.
///
/// Commands arrive stage by stage; see products/dijkstral/DIJKSTRAL_DESIGN.md.
#[derive(Parser, Debug)]
#[command(name = "dijkstral", version, about, long_about = None)]
struct Cli {}

fn main() {
    Cli::parse();
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
