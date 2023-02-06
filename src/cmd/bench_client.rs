use std::{
    fs::File,
    sync::Arc,
    time::{Duration, Instant},
};

use clap::Parser;
use ext_proc_playground::{
    dummy::{
        client::{error::StreamHandleError, ClientStream, Config},
        DummyData, DummyDataConfig,
    },
    proto::envoy::service::ext_proc::v3::external_processor_client::ExternalProcessorClient,
};

use log::{error, info};
use metered::{clear::Clear, ErrorCount, ResponseTime, Throughput};
use tokio::sync::oneshot::{self, error::TryRecvError};
use tonic::transport::Channel;

#[derive(Parser, Debug)]

struct Args {
    #[command(flatten)]
    bench_config: BenchConfig,

    /// How many threads to handle streams
    #[arg(short, default_value_t = 2)]
    thread_count: usize,

    /// Benchmark warmup duration
    #[arg(short, default_value_t = 5)]
    warmup: u64,

    /// Benchmark duration
    #[arg(short, default_value_t = 30)]
    duration: u64,

    /// Path to benchmark data json config file (see [`ext_proc_playground::dummy::DataConfig`])
    data_config_path: String,

    /// URL to External Processor gRPC Service
    #[arg(default_value = "http://[::1]:50051")]
    server_url: String,
}

#[derive(clap::Args, Debug)]
struct BenchConfig {
    /// How many streams to handle concurrently
    #[arg(short, default_value_t = 100)]
    stream_concurrency: usize,

    /// Reuse streams for more than one transaction
    #[arg(long)]
    reuse_streams: bool,

    /// How many transactions should a stream handle before closing
    #[arg(long)]
    stream_max_handle: Option<usize>,

    /// Print errors from stream handlers
    #[arg(long)]
    print_errors: bool,
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let args: Args = Args::parse();
    info!("Args:\n{:?}", args);

    let dummy_data = {
        let config_file = File::open(args.data_config_path);
        if let Err(e) = config_file {
            error!("Could not open config file: {}", e);
            return;
        }
        let config = serde_json::from_reader::<File, DummyDataConfig>(config_file.unwrap());
        if let Err(e) = config {
            error!("Could not parse config file: {}", e);
            return;
        }
        let dummy_data = DummyData::try_from(config.unwrap());
        if let Err(e) = dummy_data {
            error!("Could not initialize dummy data: {}", e);
            return;
        }
        Arc::new(dummy_data.unwrap())
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(args.thread_count)
        .enable_all()
        .build()
        .unwrap();
    let client = {
        let client = runtime.block_on(async move {
            ExternalProcessorClient::connect(args.server_url.clone()).await
        });
        if let Err(e) = client {
            error!("Could not connect to server: {}", e);
            return;
        }
        client.unwrap()
    };
    let metrics = Arc::new(StreamMetrics::default());

    let mut benchers = Vec::with_capacity(args.bench_config.stream_concurrency);
    let warmup_barrier = Arc::new(tokio::sync::Barrier::new(
        args.bench_config.stream_concurrency,
    ));
    let after_barrier = Arc::new(tokio::sync::OnceCell::new());
    for _ in 0..args.bench_config.stream_concurrency {
        benchers.push(StreamBencher {
            metrics: metrics.clone(),
            client: client.clone(),
            stream: ClientStream::new(
                dummy_data.clone(),
                Config {
                    reuse_stream: args.bench_config.reuse_streams,
                    max_handled: args.bench_config.stream_max_handle,
                },
            ),
            print_errors: args.bench_config.print_errors,
            warmup_barrier: warmup_barrier.clone(),
            after_warmup: after_barrier.clone(),
        })
    }

    runtime.block_on(perform_benchmark(
        Duration::from_secs(args.warmup),
        Duration::from_secs(args.duration),
        benchers,
        metrics,
    ));
}

async fn perform_benchmark(
    warmup: Duration,
    duration: Duration,
    benchers: Vec<StreamBencher>,
    metrics: Arc<StreamMetrics>,
) {
    const MONITOR_INTERVAL: Duration = Duration::from_secs(2);
    let (stop_metrics, stop_receiver) = tokio::sync::oneshot::channel();

    if warmup.saturating_add(duration) > MONITOR_INTERVAL {
        tokio::spawn(monitor_metrics(
            metrics.clone(),
            MONITOR_INTERVAL,
            stop_receiver,
        ));
    }

    let mut join_set = tokio::task::JoinSet::new();
    for bencher in benchers {
        join_set.spawn(bencher.bench_with_warmup(warmup, duration));
    }
    while join_set.join_next().await.is_some() {}
    _ = stop_metrics.send(());

    info!("Benchmark finished.");
    let serialized = serde_json::to_string_pretty(metrics.as_ref());
    if let Err(e) = serialized {
        error!("Could not serialize final results: {}", e);
        return;
    }
    info!("Final Results:\n{}", serialized.unwrap());
}

async fn monitor_metrics(
    metrics: Arc<StreamMetrics>,
    interval: Duration,
    mut stop: oneshot::Receiver<()>,
) {
    let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
    loop {
        interval.tick().await;
        match stop.try_recv() {
            Ok(_) | Err(TryRecvError::Closed) => break,
            Err(TryRecvError::Empty) => {}
        }
        print_metrics(metrics.as_ref());
    }
}

fn print_metrics(metrics: &StreamMetrics) {
    let err_count = metrics.run_stream.error_count.get();
    let throughput = metrics.run_stream.throughput.histogram();
    let response_time = metrics.run_stream.response_time.histogram();
    info!(
        "{:.2} req/s, {:.2}ms avg latency, {} errors",
        throughput.mean(),
        response_time.mean(),
        err_count
    );
}

struct StreamBencher {
    metrics: Arc<StreamMetrics>,

    client: ExternalProcessorClient<Channel>,
    stream: ClientStream,
    print_errors: bool,

    warmup_barrier: Arc<tokio::sync::Barrier>,
    after_warmup: Arc<tokio::sync::OnceCell<()>>,
}

impl StreamBencher {
    async fn bench_with_warmup(mut self, warmup: Duration, duration: Duration) {
        self.bench(warmup).await;
        self.warmup_barrier.wait().await;
        self.after_warmup
            .get_or_init(|| async {
                if !warmup.is_zero() {
                    info!("Warmup done.");
                }
                self.metrics.clear();
                info!("Benchmarking for {} seconds", duration.as_secs());
            })
            .await;
        self.bench(duration).await;
    }

    async fn bench(&mut self, duration: Duration) {
        let start = Instant::now();
        while start.elapsed() < duration {
            let result =
                StreamBencher::run_stream(&self.metrics, &mut self.stream, &mut self.client).await;
            match result {
                Err(e) if self.print_errors => {
                    error!("while running stream: {}", e);
                }
                _ => {}
            }
        }
    }
}

#[metered::metered(registry = StreamMetrics, registry_expr = metrics)]
impl StreamBencher {
    #[measure([ResponseTime, Throughput, ErrorCount])]
    async fn run_stream(
        metrics: &StreamMetrics,
        stream: &mut ClientStream,
        client: &mut ExternalProcessorClient<Channel>,
    ) -> Result<(), StreamHandleError> {
        stream.start_stream(client).await?;
        stream.handle_stream().await?;
        stream.finish_stream();
        Ok(())
    }
}