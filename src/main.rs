use clap::Parser;
use law_of_cycles::cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let diagnostic = cli.log;
    let result = tokio::select! {
        result = cli.run() => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };
    let Some(result) = result else {
        std::process::exit(130);
    };
    match result {
        Ok(()) => {}
        Err(error) => {
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
            {
                return;
            }
            eprintln!("kami: {}", law_of_cycles::model::clean(&error.to_string()));
            if diagnostic {
                eprintln!("diagnostic: operation failed; credentials and response bodies omitted");
            }
            std::process::exit(1);
        }
    }
}
