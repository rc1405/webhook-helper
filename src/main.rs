use clap::{Args, Parser};
use futures::join;

mod admission;
mod bootstrap;
mod controller;
mod crd;
mod operator;

#[derive(Parser)]
#[command(name = "webhook-helper")]
#[command(bin_name = "webhook-helper")]
enum WebHookHelperCli {
    Bootstrap(BootstrapArgs),
    Run(RunArgs),
}

#[derive(Args)]
#[command(author, version, about, long_about = None)]
struct BootstrapArgs {
    #[arg(short, long)]
    namespace: String,
}

#[derive(Args)]
#[command(author, version, about, long_about = None)]
pub struct RunArgs {
    #[arg(short, long)]
    port: u16,
}

/// something to drive the controller
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    match WebHookHelperCli::parse() {
        WebHookHelperCli::Bootstrap(args) => bootstrap::bootstrap(args.namespace).await?,
        WebHookHelperCli::Run(args) => {
            let adm_proc = admission::serve(args.port);
            let controller_proc = controller::run();
            let (adm_result, controller_result) = join!(adm_proc, controller_proc);
            adm_result?;
            controller_result?;
        }
    };

    Ok(())
}
