mod bootstrap;
mod config;
mod http;
mod invites;
mod migrate;
#[expect(
    dead_code,
    reason = "Route metadata is also read by the HTTP contract sweep"
)]
mod route_registry;
mod sessions;
mod shutdown;
mod telemetry;

const USAGE: &str = "Usage: fukulow [COMMAND]

Commands:
  (none)     Run the server. Reads DATABASE_URL, never MIGRATOR_DATABASE_URL
  bootstrap  Create the first organization and owner from terminal prompts
  migrate    Apply the embedded migrations with MIGRATOR_DATABASE_URL, then exit
  help       Print this message
";

#[tokio::main]
async fn main() -> anyhow::Result<std::process::ExitCode> {
    let mut args = std::env::args_os().skip(1);
    let command = args.next();
    if args.next().is_some() {
        return Err(anyhow::anyhow!("Too many arguments; run `fukulow help`"));
    }
    match command.as_deref().map(|command| command.to_str()) {
        None => serve().await.map(|()| std::process::ExitCode::SUCCESS),
        Some(Some("bootstrap")) => bootstrap::run().await,
        Some(Some("migrate")) => {
            telemetry::init()?;
            migrate::run()
                .await
                .map(|()| std::process::ExitCode::SUCCESS)
        }
        Some(Some("help" | "--help" | "-h")) => {
            usage();
            Ok(std::process::ExitCode::SUCCESS)
        }
        // The argument is not echoed: a mistyped command line can hold a secret.
        Some(_) => Err(anyhow::anyhow!("Unknown command; run `fukulow help`")),
    }
}

#[expect(
    clippy::print_stdout,
    reason = "help is the command-line interface's own output, not diagnostics"
)]
fn usage() {
    print!("{USAGE}");
}

async fn serve() -> anyhow::Result<()> {
    telemetry::init()?;
    let address = config::bind_address()?;
    let origin = config::public_origin()?;
    let pool = db::connect(&config::database_url()?).await?;
    let result = http::run(
        address,
        http::routes(sessions::StateData::new(pool.clone(), origin)),
    )
    .await;
    pool.close().await;
    result
}
