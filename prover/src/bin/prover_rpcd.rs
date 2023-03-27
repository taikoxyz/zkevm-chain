use std::env::var;

use clap::Parser;
use env_logger::Env;

use prover::server::serve;
use prover::shared_state::SharedState;

#[derive(Parser, Debug)]
#[clap(version, about)]
/// This command starts a http/json-rpc server and serves proof oriented methods.
pub(crate) struct ProverdConfig {
    #[clap(long, env = "PROVERD_BIND")]
    /// The interface address + port combination to accept connections on,
    /// e.g. `[::]:1234`.
    bind: String,
    #[clap(long, env = "PROVERD_LOOKUP")]
    /// A `HOSTNAME:PORT` conformant string that will be used for DNS service discovery of other nodes.
    lookup: Option<String>,
}

#[tokio::main]
async fn main() {
    let config = ProverdConfig::parse();
    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));
    builder.target(env_logger::Target::Stdout);
    builder.init();

    let max_tasks: usize = var("MAX_TASKS")
        .unwrap_or_else(|_| "240".to_string())
        .parse()
        .expect("Cannot parse MAX_TASKS env var as usize");

    let full_node: bool = var("FULL_NODE")
        .unwrap_or_else(|_| "true".to_string())
        .parse()
        .expect("Cannot parse FULL_NODE env var as bool");

    let shared_state = SharedState::new(
        SharedState::random_worker_id(),
        config.lookup,
        max_tasks,
        full_node,
    );
    {
        // start the http server
        let h1 = serve(&shared_state, &config.bind);

        // full_node does not have duty_cycle as it merges results only
        if !full_node {
            // starts the duty cycle loop
            let ctx = shared_state.clone();
            // use a dedicated runtime for mixed async / heavy (blocking) compute
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let h2 = rt.spawn(async move {
                loop {
                    let ctx = ctx.clone();
                    // enclose this call to catch panics which may
                    // occur due to network services
                    let _ = tokio::spawn(async move {
                        log::debug!("task: duty_cycle");
                        ctx.duty_cycle().await;
                    })
                    .await;
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                }
            });
            // wait for all tasks
            if tokio::try_join!(h1, h2).is_err() {
                panic!("unexpected task error");
            }
        } else {
            // starts the duty cycle loop
            let ctx = shared_state.clone();
            // use a dedicated runtime for mixed async / heavy (blocking) compute
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            let h2 = rt.spawn(async move {
                loop {
                    let ctx = ctx.clone();
                    // enclose this call to catch panics which may
                    // occur due to network services
                    let _ = tokio::spawn(async move {
                        log::debug!("task: dispatch_task_duty_cycle");
                        let _ = ctx.dispatch_tasks_to_peers().await;
                    })
                    .await;
                    tokio::time::sleep(std::time::Duration::from_millis(5000)).await;
                }
            });

            // this task loop makes sure to merge task results periodically
            // even if this instance is busy with proving
            let ctx = shared_state.clone();
            let h3 = tokio::spawn(async move {
                loop {
                    let ctx = ctx.clone();
                    // enclose this call to catch panics which may
                    // occur due to network services
                    let _ = tokio::spawn(async move {
                        log::debug!("task: merge_tasks_from_peers");
                        let _ = ctx.merge_tasks_from_peers().await;
                    })
                    .await;
                    tokio::time::sleep(std::time::Duration::from_millis(10000)).await;
                }
            });

            // wait for all tasks
            if tokio::try_join!(h1, h2, h3).is_err() {
                panic!("unexpected task error");
            }
        };
    }
}
