use clap::Parser;
use ext_proc_playground::{
    dummy::server::ExtProcService,
    proto::envoy::{
        extensions::filters::http::ext_proc::v3::{
            processing_mode::{BodySendMode, HeaderSendMode},
            ProcessingMode,
        },
        service::ext_proc::v3::external_processor_server::ExternalProcessorServer,
    },
};
use log::{error, info};
use tonic::transport::Server;

#[derive(Parser, Debug)]
struct Args {
    /// Server worker threads
    #[arg(short, default_value_t = 2)]
    thread_count: usize,

    // Port to listen to
    #[arg(short, default_value_t = 50051)]
    port: u16,
}

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    let args = Args::parse();

    let mut processing_mode = ProcessingMode::default();
    processing_mode.set_request_header_mode(HeaderSendMode::Send);
    processing_mode.set_response_header_mode(HeaderSendMode::Send);
    processing_mode.set_request_body_mode(BodySendMode::Buffered);
    processing_mode.set_response_body_mode(BodySendMode::Buffered);
    processing_mode.set_request_trailer_mode(HeaderSendMode::Skip);
    processing_mode.set_response_trailer_mode(HeaderSendMode::Skip);

    let service = ExtProcService::new(processing_mode);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(args.thread_count)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let address = format!("[::1]:{}", args.port);
        info!("Serving {}", address);
        let server = ExternalProcessorServer::new(service);
        if let Err(e) = Server::builder()
            .add_service(server)
            .serve(format!("[::1]:{}", args.port).parse().unwrap())
            .await
        {
            error!("error serving gRPC: {}", e);
        }
    });
}
